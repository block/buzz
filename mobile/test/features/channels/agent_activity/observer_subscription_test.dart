import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/channels/agent_activity/observer_models.dart';
import 'package:buzz/features/channels/agent_activity/observer_subscription.dart';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  test('decodes explicit root scope separately from legacy omission', () {
    final explicitRoot = ObserverFrame.fromJson({
      'seq': 1,
      'timestamp': '2026-04-30T12:00:01.000Z',
      'kind': 'turn_liveness',
      'channelId': 'channel-1',
      'threadHeadId': null,
      'turnId': 'turn-1',
    });
    final legacy = ObserverFrame.fromJson({
      'seq': 2,
      'timestamp': '2026-04-30T12:00:02.000Z',
      'kind': 'turn_liveness',
      'channelId': 'channel-1',
      'turnId': 'turn-1',
    });

    expect(explicitRoot.hasThreadScope, isTrue);
    expect(explicitRoot.threadHeadId, isNull);
    expect(legacy.hasThreadScope, isFalse);
  });

  test('turn-scoped provider does not merge concurrent agent turns', () {
    final container = ProviderContainer(
      overrides: [
        observerRelayProvider.overrideWith(
          () => _StaticObserverRelay({
            'agent-a': [
              _turnMessageFrame(seq: 1, turnId: 'turn-1', text: 'First turn'),
              _turnMessageFrame(seq: 2, turnId: 'turn-2', text: 'Second turn'),
              ObserverFrame(
                seq: 3,
                timestamp: '2026-04-30T12:00:03.000Z',
                kind: 'agent_panic',
                turnId: 'turn-1',
                payload: const {'error': 'Process exited'},
              ),
            ],
          }),
        ),
      ],
    );
    addTearDown(container.dispose);

    final state = container.read(
      observerTurnSubscriptionProvider((
        channelId: 'test-channel',
        agentPubkey: 'agent-a',
        turnId: 'turn-1',
      )),
    );

    expect(state.transcript.whereType<MessageItem>().single.text, 'First turn');
    expect(
      state.transcript.whereType<LifecycleItem>().single.title,
      'Agent error',
    );
  });

  test('provider initializes without circular dependency error', () {
    // Regression test: reading the provider should NOT throw
    // "Bad state: Tried to read the state of an uninitialized provider".
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => _RecordingRelaySession()),
        relayConfigProvider.overrideWith(
          () => _FakeRelayConfigNotifier(nsec: null),
        ),
      ],
    );
    addTearDown(container.dispose);

    const key = (channelId: 'test-channel', agentPubkey: 'deadbeef');
    // This line threw before the fix.
    final state = container.read(observerSubscriptionProvider(key));
    expect(state.connection, ObserverConnectionState.idle);
    expect(state.transcript, isEmpty);
  });

  test('transitions to error when nsec is invalid', () async {
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => _RecordingRelaySession()),
        relayConfigProvider.overrideWith(
          () => _FakeRelayConfigNotifier(nsec: 'nsec1invalid'),
        ),
      ],
    );
    addTearDown(container.dispose);

    const key = (channelId: 'test-channel', agentPubkey: 'deadbeef');
    container.read(observerSubscriptionProvider(key));

    // Let the subscription microtask run.
    await Future<void>.delayed(Duration.zero);

    final state = container.read(observerSubscriptionProvider(key));
    // With invalid nsec, it should be in error state, NOT throw.
    expect(state.connection, ObserverConnectionState.error);
    expect(state.errorMessage, isNotNull);
  });

  test('stays idle when nsec is null', () async {
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => _RecordingRelaySession()),
        relayConfigProvider.overrideWith(
          () => _FakeRelayConfigNotifier(nsec: null),
        ),
      ],
    );
    addTearDown(container.dispose);

    const key = (channelId: 'test-channel', agentPubkey: 'deadbeef');
    container.read(observerSubscriptionProvider(key));

    // Let the subscription microtask run. Without an nsec, it should no-op.
    await Future<void>.delayed(Duration.zero);

    final state = container.read(observerSubscriptionProvider(key));
    expect(state.connection, ObserverConnectionState.idle);
    expect(state.transcript, isEmpty);
  });

  test(
    'subscribes with correct filter shape and transitions to open',
    () async {
      final userKeychain = nostr.Keys.generate();
      final nsec = userKeychain.nsec;
      final myPubkey = userKeychain.public;
      // Agent needs a valid 64-char hex pubkey for getConversationKey.
      final agentKeychain = nostr.Keys.generate();
      final agentPubkey = agentKeychain.public;

      final relaySession = _RecordingRelaySession();
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => relaySession),
          relayConfigProvider.overrideWith(
            () => _FakeRelayConfigNotifier(nsec: nsec),
          ),
        ],
      );
      addTearDown(container.dispose);

      final key = (channelId: 'test-channel', agentPubkey: agentPubkey);
      container.read(observerSubscriptionProvider(key));

      // Let the subscription microtask run.
      await Future<void>.delayed(Duration.zero);

      final state = container.read(observerSubscriptionProvider(key));
      expect(state.connection, ObserverConnectionState.open);
      expect(state.transcript, isEmpty);

      // Verify the subscription filter shape.
      expect(relaySession.filters, hasLength(1));
      final filter = relaySession.filters.first;
      expect(filter.kinds, [EventKind.agentObserverFrame]);
      expect(filter.limit, 0);
      expect(filter.tags['#p'], contains(myPubkey));
      expect(filter.since, isNull);
    },
  );

  test(
    'uses one shared relay subscription for channel-scoped readers',
    () async {
      final userKeychain = nostr.Keys.generate();
      final agentKeychain = nostr.Keys.generate();
      final relaySession = _RecordingRelaySession();
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => relaySession),
          relayConfigProvider.overrideWith(
            () => _FakeRelayConfigNotifier(nsec: userKeychain.nsec),
          ),
        ],
      );
      addTearDown(container.dispose);

      container.read(
        observerSubscriptionProvider((
          channelId: 'first-channel',
          agentPubkey: agentKeychain.public,
        )),
      );
      container.read(
        observerSubscriptionProvider((
          channelId: 'second-channel',
          agentPubkey: agentKeychain.public,
        )),
      );

      await Future<void>.delayed(Duration.zero);

      expect(relaySession.filters, hasLength(1));
    },
  );

  test('surfaces relay CLOSED messages through observer state', () async {
    final userKeychain = nostr.Keys.generate();
    final agentKeychain = nostr.Keys.generate();
    final relaySession = _RecordingRelaySession();
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => relaySession),
        relayConfigProvider.overrideWith(
          () => _FakeRelayConfigNotifier(nsec: userKeychain.nsec),
        ),
      ],
    );
    addTearDown(container.dispose);

    final key = (channelId: 'test-channel', agentPubkey: agentKeychain.public);
    container.read(observerSubscriptionProvider(key));
    await Future<void>.delayed(Duration.zero);

    relaySession.closeAll('restricted: p-gated events require #p');

    final state = container.read(observerSubscriptionProvider(key));
    expect(state.connection, ObserverConnectionState.error);
    expect(state.errorMessage, contains('p-gated events require #p'));
  });

  test('ignores stale subscribe completion after identity changes', () async {
    final firstUser = nostr.Keys.generate();
    final secondUser = nostr.Keys.generate();
    final agentKeychain = nostr.Keys.generate();
    final relaySession = _RecordingRelaySession()..delaySubscribes = true;
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => relaySession),
        relayConfigProvider.overrideWith(
          () => _FakeRelayConfigNotifier(nsec: firstUser.nsec),
        ),
      ],
    );
    addTearDown(container.dispose);

    final key = (channelId: 'test-channel', agentPubkey: agentKeychain.public);
    container.read(observerSubscriptionProvider(key));
    await Future<void>.delayed(Duration.zero);

    expect(relaySession.filters, hasLength(1));
    expect(relaySession.filters.single.tags['#p'], [firstUser.public]);

    (container.read(relayConfigProvider.notifier) as _FakeRelayConfigNotifier)
        .setNsec(secondUser.nsec);
    container.read(observerSubscriptionProvider(key));
    await Future<void>.delayed(Duration.zero);

    expect(relaySession.filters, hasLength(2));
    expect(relaySession.filters.last.tags['#p'], [secondUser.public]);

    relaySession.releaseSubscribe(0);
    await Future<void>.delayed(Duration.zero);

    expect(relaySession.filters, hasLength(1));
    expect(relaySession.filters.single.tags['#p'], [secondUser.public]);

    relaySession.releaseSubscribe(1);
    await Future<void>.delayed(Duration.zero);

    final state = container.read(observerSubscriptionProvider(key));
    expect(state.connection, ObserverConnectionState.open);
    expect(relaySession.filters, hasLength(1));
  });

  test(
    'decrypts observer frames and exposes channel-scoped transcript',
    () async {
      final ownerKeychain = nostr.Keys.generate();
      final agentKeychain = nostr.Keys.generate();
      final nsec = ownerKeychain.nsec;
      final relaySession = _RecordingRelaySession();
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => relaySession),
          relayConfigProvider.overrideWith(
            () => _FakeRelayConfigNotifier(nsec: nsec),
          ),
        ],
      );
      addTearDown(container.dispose);

      const channelId = 'test-channel';
      final key = (channelId: channelId, agentPubkey: agentKeychain.public);
      container.read(observerSubscriptionProvider(key));
      await Future<void>.delayed(Duration.zero);

      final conversationKey = getConversationKey(
        agentKeychain.secret,
        ownerKeychain.public,
      );
      final encrypted = nip44Encrypt(
        conversationKey,
        jsonEncode({
          'seq': 1,
          'timestamp': '2026-04-30T12:00:00.000Z',
          'kind': 'turn_started',
          'channelId': channelId,
          'turnId': 'turn-1',
          'startedAt': '2026-04-30T11:59:00.000Z',
          'payload': {
            'triggeringEventIds': ['0123456789abcdef'],
          },
        }),
      );
      final event = nostr.Event.from(
        kind: EventKind.agentObserverFrame,
        content: encrypted,
        tags: [
          ['p', ownerKeychain.public],
          ['agent', agentKeychain.public],
          ['frame', 'telemetry'],
        ],
        secretKey: agentKeychain.secret,
        verify: false,
      );

      relaySession.emit(NostrEvent.fromJson(event.toMap()));

      final state = container.read(observerSubscriptionProvider(key));
      expect(state.connection, ObserverConnectionState.open);
      expect(state.transcript, hasLength(1));
      final item = state.transcript.single;
      expect(item, isA<LifecycleItem>());
      expect((item as LifecycleItem).title, 'Turn started');
      final storedFrame = container
          .read(observerRelayProvider)
          .framesByAgent[agentKeychain.public]!
          .single;
      expect(storedFrame.startedAt, '2026-04-30T11:59:00.000Z');
      expect(storedFrame.receivedAt, isNotNull);

      final otherChannelState = container.read(
        observerSubscriptionProvider((
          channelId: 'other-channel',
          agentPubkey: agentKeychain.public,
        )),
      );
      expect(otherChannelState.transcript, isEmpty);
    },
  );

  test('expands batch envelopes through ordering and dedupe', () async {
    final ownerKeychain = nostr.Keys.generate();
    final agentKeychain = nostr.Keys.generate();
    final relaySession = _RecordingRelaySession();
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => relaySession),
        relayConfigProvider.overrideWith(
          () => _FakeRelayConfigNotifier(nsec: ownerKeychain.nsec),
        ),
      ],
    );
    addTearDown(container.dispose);

    const channelId = 'test-channel';
    final key = (channelId: channelId, agentPubkey: agentKeychain.public);
    container.read(observerSubscriptionProvider(key));
    await Future<void>.delayed(Duration.zero);

    final laterFrame = _observerFrameJson(
      seq: 2,
      channelId: channelId,
      turnId: 'turn-2',
    )..['threadHeadId'] = null;
    final earlierFrame = _observerFrameJson(
      seq: 1,
      channelId: channelId,
      threadHeadId: 'thread-1',
      turnId: 'turn-1',
    );
    relaySession.emit(
      _observerEvent(
        ownerKeychain: ownerKeychain,
        agentKeychain: agentKeychain,
        payload: {
          'seq': 2,
          'timestamp': '2026-04-30T12:00:02.000Z',
          'kind': 'batch',
          'channelId': channelId,
          'turnId': 'turn-2',
          'payload': {
            'events': [laterFrame, earlierFrame, earlierFrame],
          },
        },
      ),
    );

    final relayState = container.read(observerRelayProvider);
    final frames = relayState.framesByAgent[agentKeychain.public];
    expect(frames?.map((frame) => frame.seq), [1, 2]);
    expect(
      frames![0].receivedAt!.isBefore(frames[1].receivedAt!),
      isTrue,
      reason: 'batch receipt order must follow timestamp and sequence',
    );
    expect(frames[0].threadHeadId, 'thread-1');
    expect(frames[0].hasThreadScope, isTrue);
    expect(frames[1].threadHeadId, isNull);
    expect(frames[1].hasThreadScope, isTrue);

    final state = container.read(observerSubscriptionProvider(key));
    expect(state.connection, ObserverConnectionState.open);
    expect(state.transcript, hasLength(2));
    expect(state.transcript.map((item) => item.id), [
      'turn:turn-1',
      'turn:turn-2',
    ]);

    final otherChannelState = container.read(
      observerSubscriptionProvider((
        channelId: 'other-channel',
        agentPubkey: agentKeychain.public,
      )),
    );
    expect(otherChannelState.transcript, isEmpty);
  });

  test('publishes one open-state update for a changed batch', () async {
    final ownerKeychain = nostr.Keys.generate();
    final agentKeychain = nostr.Keys.generate();
    final relaySession = _RecordingRelaySession();
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => relaySession),
        relayConfigProvider.overrideWith(
          () => _FakeRelayConfigNotifier(nsec: ownerKeychain.nsec),
        ),
      ],
    );
    addTearDown(container.dispose);

    container.read(
      observerSubscriptionProvider((
        channelId: 'test-channel',
        agentPubkey: agentKeychain.public,
      )),
    );
    await Future<void>.delayed(Duration.zero);

    var openStateUpdates = 0;
    final listener = container.listen(observerRelayProvider, (_, next) {
      if (next.connection == ObserverConnectionState.open) {
        openStateUpdates += 1;
      }
    });
    addTearDown(listener.close);

    final event = _observerEvent(
      ownerKeychain: ownerKeychain,
      agentKeychain: agentKeychain,
      payload: {
        'seq': 2,
        'timestamp': '2026-04-30T12:00:02.000Z',
        'kind': 'batch',
        'channelId': 'test-channel',
        'turnId': 'turn-2',
        'payload': {
          'events': [
            _observerFrameJson(
              seq: 1,
              channelId: 'test-channel',
              turnId: 'turn-1',
            ),
            _observerFrameJson(
              seq: 2,
              channelId: 'test-channel',
              turnId: 'turn-2',
            ),
          ],
        },
      },
    );

    relaySession.emit(event);
    expect(openStateUpdates, 1);

    relaySession.emit(event);
    expect(openStateUpdates, 1);
  });

  test('keeps malformed batch envelopes as singleton frames', () async {
    final ownerKeychain = nostr.Keys.generate();
    final agentKeychain = nostr.Keys.generate();
    final relaySession = _RecordingRelaySession();
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => relaySession),
        relayConfigProvider.overrideWith(
          () => _FakeRelayConfigNotifier(nsec: ownerKeychain.nsec),
        ),
      ],
    );
    addTearDown(container.dispose);

    container.read(
      observerSubscriptionProvider((
        channelId: 'test-channel',
        agentPubkey: agentKeychain.public,
      )),
    );
    await Future<void>.delayed(Duration.zero);

    relaySession.emit(
      _observerEvent(
        ownerKeychain: ownerKeychain,
        agentKeychain: agentKeychain,
        payload: {
          'seq': 3,
          'timestamp': '2026-04-30T12:00:03.000Z',
          'kind': 'batch',
          'channelId': 'test-channel',
          'payload': <String, dynamic>{},
        },
      ),
    );

    final state = container.read(observerRelayProvider);
    expect(state.connection, ObserverConnectionState.open);
    expect(state.errorMessage, isNull);
    expect(state.framesByAgent[agentKeychain.public], hasLength(1));
    expect(state.framesByAgent[agentKeychain.public]!.single.kind, 'batch');
  });

  test(
    'rejects invalid inner batch frames without partial ingestion',
    () async {
      final ownerKeychain = nostr.Keys.generate();
      final agentKeychain = nostr.Keys.generate();
      final relaySession = _RecordingRelaySession();
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => relaySession),
          relayConfigProvider.overrideWith(
            () => _FakeRelayConfigNotifier(nsec: ownerKeychain.nsec),
          ),
        ],
      );
      addTearDown(container.dispose);

      container.read(
        observerSubscriptionProvider((
          channelId: 'test-channel',
          agentPubkey: agentKeychain.public,
        )),
      );
      await Future<void>.delayed(Duration.zero);

      relaySession.emit(
        _observerEvent(
          ownerKeychain: ownerKeychain,
          agentKeychain: agentKeychain,
          payload: {
            'seq': 2,
            'timestamp': '2026-04-30T12:00:02.000Z',
            'kind': 'batch',
            'channelId': 'test-channel',
            'payload': {
              'events': [
                _observerFrameJson(
                  seq: 1,
                  channelId: 'test-channel',
                  turnId: 'turn-1',
                ),
                {'seq': 'invalid'},
              ],
            },
          },
        ),
      );

      final state = container.read(observerRelayProvider);
      expect(state.connection, ObserverConnectionState.error);
      expect(state.errorMessage, contains('Observer event decrypt failed'));
      expect(state.framesByAgent[agentKeychain.public], isNull);
    },
  );
}

ObserverFrame _turnMessageFrame({
  required int seq,
  required String turnId,
  required String text,
}) => ObserverFrame(
  seq: seq,
  timestamp: '2026-04-30T12:00:0$seq.000Z',
  kind: 'acp_read',
  channelId: 'test-channel',
  turnId: turnId,
  payload: {
    'method': 'session/update',
    'params': {
      'update': {
        'sessionUpdate': 'agent_message_chunk',
        'messageId': 'message-$turnId',
        'content': [
          {'type': 'text', 'text': text},
        ],
      },
    },
  },
);

Map<String, dynamic> _observerFrameJson({
  required int seq,
  required String channelId,
  String? threadHeadId,
  required String turnId,
}) => {
  'seq': seq,
  'timestamp': '2026-04-30T12:00:0$seq.000Z',
  'kind': 'turn_started',
  'channelId': channelId,
  'threadHeadId': ?threadHeadId,
  'turnId': turnId,
  'payload': {
    'triggeringEventIds': ['$seq'],
  },
};

NostrEvent _observerEvent({
  required nostr.Keys ownerKeychain,
  required nostr.Keys agentKeychain,
  required Map<String, dynamic> payload,
}) {
  final conversationKey = getConversationKey(
    agentKeychain.secret,
    ownerKeychain.public,
  );
  final event = nostr.Event.from(
    kind: EventKind.agentObserverFrame,
    content: nip44Encrypt(conversationKey, jsonEncode(payload)),
    tags: [
      ['p', ownerKeychain.public],
      ['agent', agentKeychain.public],
      ['frame', 'telemetry'],
    ],
    secretKey: agentKeychain.secret,
    verify: false,
  );
  return NostrEvent.fromJson(event.toMap());
}

class _RecordingRelaySession extends RelaySessionNotifier {
  final List<NostrFilter> filters = [];
  final List<void Function(NostrEvent)> _listeners = [];
  final List<void Function(String message)> _closedListeners = [];
  final List<Completer<void>> _subscribeGates = [];
  bool delaySubscribes = false;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    filters.add(filter);
    _listeners.add(onEvent);
    if (onClosed != null) {
      _closedListeners.add(onClosed);
    }
    if (delaySubscribes) {
      final gate = Completer<void>();
      _subscribeGates.add(gate);
      await gate.future;
    }
    return () {
      filters.remove(filter);
      _listeners.remove(onEvent);
      if (onClosed != null) {
        _closedListeners.remove(onClosed);
      }
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

  void releaseSubscribe(int index) {
    final gate = _subscribeGates[index];
    if (!gate.isCompleted) {
      gate.complete();
    }
  }
}

class _StaticObserverRelay extends ObserverRelayNotifier {
  final Map<String, List<ObserverFrame>> frames;

  _StaticObserverRelay(this.frames);

  @override
  ObserverRelayState build() => ObserverRelayState(
    connection: ObserverConnectionState.open,
    framesByAgent: frames,
  );
}

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  String? _nsec;

  _FakeRelayConfigNotifier({required String? nsec}) : _nsec = nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'http://localhost:3000', nsec: _nsec);

  void setNsec(String? nsec) {
    _nsec = nsec;
    state = RelayConfig(baseUrl: 'http://localhost:3000', nsec: _nsec);
  }
}
