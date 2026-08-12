import 'dart:convert';

import 'package:buzz/features/channels/agent_activity/shared_activity_subscription.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

const _channelId = 'ab7351d0-59fd-4f30-b1d0-3e2754b66a50';
final _now = DateTime.utc(2026, 8, 12, 12);

void main() {
  test('uses a separate live-only channel and agent subscription', () async {
    final firstAgent = nostr.Keys.generate();
    final secondAgent = nostr.Keys.generate();
    final session = _RecordingRelaySession();
    final container = _container(session);
    addTearDown(container.dispose);

    final firstKey = (channelId: _channelId, agentPubkey: firstAgent.public);
    const secondChannel = '3caf753b-8e2b-4e59-81c6-6e9b962c459c';
    final secondKey = (
      channelId: secondChannel,
      agentPubkey: secondAgent.public,
    );
    final firstKeepAlive = container.listen(
      sharedActivitySubscriptionProvider(firstKey),
      (_, _) {},
      fireImmediately: true,
    );
    final secondKeepAlive = container.listen(
      sharedActivitySubscriptionProvider(secondKey),
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(firstKeepAlive.close);
    addTearDown(secondKeepAlive.close);

    await Future<void>.delayed(Duration.zero);

    expect(session.filters, hasLength(2));
    expect(session.filters[0].kinds, [EventKind.agentActivitySummary]);
    expect(session.filters[0].authors, [firstAgent.public]);
    expect(session.filters[0].tags, {
      '#h': [_channelId],
    });
    expect(session.filters[0].limit, 0);
    expect(session.filters[0].since, isNull);
    expect(session.filters[0].until, isNull);
    expect(session.filters[1].authors, [secondAgent.public]);
    expect(session.filters[1].tags, {
      '#h': [secondChannel],
    });
    expect(session.historyCalls, 0);
  });

  test('accepts a fresh fully verified exact summary event', () async {
    final agent = nostr.Keys.generate();
    final session = _RecordingRelaySession();
    final container = _container(session);
    addTearDown(container.dispose);
    final key = (channelId: _channelId, agentPubkey: agent.public);
    final keepAlive = container.listen(
      sharedActivitySubscriptionProvider(key),
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(keepAlive.close);
    await Future<void>.delayed(Duration.zero);

    expect(
      container.read(sharedActivitySubscriptionProvider(key)).connection,
      SharedActivityConnectionState.live,
    );

    session.emit(_signedEvent(agent, activities: [_activity()]));

    final state = container.read(sharedActivitySubscriptionProvider(key));
    expect(state.connection, SharedActivityConnectionState.live);
    expect(state.activities, hasLength(1));
    expect(state.activities.single.activityId, _activityId(0));
  });

  test('rejects unverified, mis-scoped, malformed, and stale events', () async {
    final agent = nostr.Keys.generate();
    final other = nostr.Keys.generate();
    final session = _RecordingRelaySession();
    final container = _container(session);
    addTearDown(container.dispose);
    final key = (channelId: _channelId, agentPubkey: agent.public);
    final keepAlive = container.listen(
      sharedActivitySubscriptionProvider(key),
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(keepAlive.close);
    await Future<void>.delayed(Duration.zero);

    final valid = _signedEvent(agent, activities: [_activity()]);
    final malformedEvents = <NostrEvent>[
      NostrEvent(
        id: '0' * 64,
        pubkey: valid.pubkey,
        createdAt: valid.createdAt,
        kind: valid.kind,
        tags: valid.tags,
        content: valid.content,
        sig: valid.sig,
      ),
      NostrEvent(
        id: valid.id,
        pubkey: valid.pubkey,
        createdAt: valid.createdAt,
        kind: valid.kind,
        tags: valid.tags,
        content: valid.content,
        sig: '0' * 128,
      ),
      _signedEvent(other, activities: [_activity()]),
      _signedEvent(
        agent,
        activities: [_activity()],
        tags: [
          ['h', _channelId],
          ['agent', other.public],
        ],
      ),
      _signedEvent(
        agent,
        activities: [_activity()],
        tags: [
          ['h', '3caf753b-8e2b-4e59-81c6-6e9b962c459c'],
          ['agent', agent.public],
        ],
      ),
      _signedEvent(
        agent,
        activities: [_activity()],
        tags: [
          ['h', _channelId],
          ['h', _channelId],
          ['agent', agent.public],
        ],
      ),
      _signedEvent(
        agent,
        activities: [_activity()],
        tags: [
          ['h', _channelId, 'extended'],
          ['agent', agent.public],
        ],
      ),
      _signedEvent(
        agent,
        activities: [_activity()],
        tags: [
          ['h', _channelId],
          ['agent', agent.public, 'extended'],
        ],
      ),
      _signedEvent(
        agent,
        activities: [_activity()],
        tags: [
          ['h', _channelId],
          ['agent', agent.public],
          ['p', other.public],
        ],
      ),
      _signedEvent(
        agent,
        activities: [_activity()],
        createdAt: _seconds(
          _now.subtract(const Duration(minutes: 5, seconds: 1)),
        ),
      ),
      _signedEvent(
        agent,
        activities: [_activity()],
        createdAt: _seconds(_now.add(const Duration(minutes: 5, seconds: 1))),
      ),
      _signedEvent(
        agent,
        activities: [_activity()]..single['prompt'] = 'PRIVATE_PROMPT',
      ),
    ];

    for (final event in malformedEvents) {
      session.emit(event);
    }

    expect(
      container.read(sharedActivitySubscriptionProvider(key)).activities,
      isEmpty,
    );
  });

  test(
    'deduplicates replays and retains only the newest 200 activities',
    () async {
      final agent = nostr.Keys.generate();
      final session = _RecordingRelaySession();
      final container = _container(session);
      addTearDown(container.dispose);
      final key = (channelId: _channelId, agentPubkey: agent.public);
      final keepAlive = container.listen(
        sharedActivitySubscriptionProvider(key),
        (_, _) {},
        fireImmediately: true,
      );
      addTearDown(keepAlive.close);
      await Future<void>.delayed(Duration.zero);

      final replay = _signedEvent(agent, activities: [_activity()]);
      session.emit(replay);
      session.emit(replay);

      var next = 1;
      while (next <= 200) {
        final end = (next + 15).clamp(0, 200);
        session.emit(
          _signedEvent(
            agent,
            activities: [
              for (var index = next; index <= end; index++) _activity(index),
            ],
          ),
        );
        next = end + 1;
      }

      final activities = container
          .read(sharedActivitySubscriptionProvider(key))
          .activities;
      expect(activities, hasLength(200));
      expect(
        activities.map((item) => item.activityId),
        isNot(contains(_activityId(0))),
      );
      expect(activities.map((item) => item.activityId).toSet(), hasLength(200));
    },
  );

  test('surfaces terminal CLOSED and subscribe errors', () async {
    final agent = nostr.Keys.generate();
    final session = _RecordingRelaySession();
    final container = _container(session);
    addTearDown(container.dispose);
    final key = (channelId: _channelId, agentPubkey: agent.public);
    final keepAlive = container.listen(
      sharedActivitySubscriptionProvider(key),
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(keepAlive.close);
    await Future<void>.delayed(Duration.zero);

    session.emit(_signedEvent(agent, activities: [_activity()]));
    expect(
      container.read(sharedActivitySubscriptionProvider(key)).activities,
      hasLength(1),
    );

    session.closeAll('restricted: channel membership required');
    var state = container.read(sharedActivitySubscriptionProvider(key));
    expect(state.connection, SharedActivityConnectionState.closed);
    expect(state.errorMessage, contains('channel membership required'));
    expect(state.activities, isEmpty);

    final failingSession = _RecordingRelaySession()
      ..subscribeError = StateError('socket failed');
    final failingContainer = _container(failingSession);
    addTearDown(failingContainer.dispose);
    final failingKey = (channelId: _channelId, agentPubkey: agent.public);
    final failingKeepAlive = failingContainer.listen(
      sharedActivitySubscriptionProvider(failingKey),
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(failingKeepAlive.close);
    await Future<void>.delayed(Duration.zero);

    state = failingContainer.read(
      sharedActivitySubscriptionProvider(failingKey),
    );
    expect(state.connection, SharedActivityConnectionState.error);
    expect(state.errorMessage, contains('socket failed'));
  });
}

ProviderContainer _container(_RecordingRelaySession session) =>
    ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => session),
        sharedActivityNowProvider.overrideWithValue(() => _now),
      ],
    );

Map<String, Object?> _activity([int index = 0]) => {
  'activityId': _activityId(index),
  'occurredAt': _now.add(Duration(seconds: index)).toIso8601String(),
  'activityClass': 'turn',
  'status': 'started',
};

String _activityId(int index) =>
    '00000000-0000-4000-8000-${index.toString().padLeft(12, '0')}';

NostrEvent _signedEvent(
  nostr.Keys signer, {
  required List<Map<String, Object?>> activities,
  List<List<String>>? tags,
  int? createdAt,
}) {
  final event = nostr.Event.from(
    kind: EventKind.agentActivitySummary,
    content: jsonEncode({'version': 1, 'activities': activities}),
    tags:
        tags ??
        [
          ['h', _channelId],
          ['agent', signer.public],
        ],
    secretKey: signer.secret,
    createdAt: createdAt ?? _seconds(_now),
    verify: true,
  );
  return NostrEvent.fromJson(event.toMap());
}

int _seconds(DateTime value) => value.millisecondsSinceEpoch ~/ 1000;

class _RecordingRelaySession extends RelaySessionNotifier {
  final List<NostrFilter> filters = [];
  final List<void Function(NostrEvent)> _listeners = [];
  final List<void Function(String)> _closedListeners = [];
  Object? subscribeError;
  int historyCalls = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    historyCalls += 1;
    return const [];
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) => _recordSubscription(filter, onEvent, onClosed: onClosed);

  @override
  Future<void Function()> subscribeValidatedLiveOnly(
    NostrFilter filter,
    bool Function(NostrEvent) admitEvent,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) => _recordSubscription(filter, (event) {
    if (admitEvent(event)) onEvent(event);
  }, onClosed: onClosed);

  Future<void Function()> _recordSubscription(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    final error = subscribeError;
    if (error != null) throw error;
    filters.add(filter);
    _listeners.add(onEvent);
    if (onClosed != null) _closedListeners.add(onClosed);
    return () {
      filters.remove(filter);
      _listeners.remove(onEvent);
      if (onClosed != null) _closedListeners.remove(onClosed);
    };
  }

  void emit(NostrEvent event) {
    for (final listener in List.of(_listeners)) {
      listener(event);
    }
  }

  void closeAll(String message) {
    for (final listener in List.of(_closedListeners)) {
      listener(message);
    }
    filters.clear();
    _listeners.clear();
    _closedListeners.clear();
  }
}
