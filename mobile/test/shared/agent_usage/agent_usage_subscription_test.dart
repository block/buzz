import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/shared/agent_usage/agent_usage_subscription.dart';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  test('provider initializes without circular dependency error', () {
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => _RecordingRelaySession()),
        relayConfigProvider.overrideWith(
          () => _FakeRelayConfigNotifier(nsec: null),
        ),
      ],
    );
    addTearDown(container.dispose);

    final state = container.read(agentUsageRelayProvider);
    expect(state.connection, AgentUsageConnectionState.idle);
    expect(state.snapshotsByAgent, isEmpty);
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

    container.read(agentUsageRelayProvider);
    await Future<void>.delayed(Duration.zero);

    final state = container.read(agentUsageRelayProvider);
    expect(state.connection, AgentUsageConnectionState.error);
    expect(state.errorMessage, isNotNull);
  });

  test('subscribes with the NIP-AM owner-scoped filter shape', () async {
    final userKeychain = nostr.Keys.generate();
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

    container.read(agentUsageRelayProvider);
    await Future<void>.delayed(Duration.zero);

    final state = container.read(agentUsageRelayProvider);
    expect(state.connection, AgentUsageConnectionState.open);

    expect(relaySession.filters, hasLength(1));
    final filter = relaySession.filters.first;
    expect(filter.kinds, [EventKind.agentTurnMetric]);
    expect(filter.tags['#p'], [userKeychain.public]);
  });

  test('decrypts a kind:44200 event into a per-agent snapshot', () async {
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

    container.read(agentUsageRelayProvider);
    await Future<void>.delayed(Duration.zero);

    relaySession.emit(
      _turnMetricEvent(
        ownerKeychain: ownerKeychain,
        agentKeychain: agentKeychain,
        payload: {
          'harness': 'goose',
          'timestamp': '2026-07-01T20:11:03Z',
          'contextUsedTokens': 15392,
          'contextLimitTokens': 272000,
        },
      ),
    );

    final state = container.read(agentUsageRelayProvider);
    expect(state.connection, AgentUsageConnectionState.open);
    final snapshot = state.snapshotsByAgent[agentKeychain.public];
    expect(snapshot, isNotNull);
    expect(snapshot!.contextUsedTokens, 15392);
    expect(snapshot.contextLimitTokens, 272000);
  });

  test('drops an event with an invalid NIP-01 signature', () async {
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

    container.read(agentUsageRelayProvider);
    await Future<void>.delayed(Duration.zero);

    final validEvent = _turnMetricEvent(
      ownerKeychain: ownerKeychain,
      agentKeychain: agentKeychain,
      payload: {
        'harness': 'goose',
        'timestamp': '2026-07-01T20:11:03Z',
        'contextUsedTokens': 15392,
        'contextLimitTokens': 272000,
      },
    );
    relaySession.emit(
      NostrEvent.fromJson({...validEvent.toJson(), 'sig': '00' * 64}),
    );

    final state = container.read(agentUsageRelayProvider);
    expect(state.connection, AgentUsageConnectionState.open);
    expect(state.snapshotsByAgent, isEmpty);
    expect(state.errorMessage, isNull);
  });

  test('drops an event with the wrong p tag without erroring', () async {
    final ownerKeychain = nostr.Keys.generate();
    final otherOwnerKeychain = nostr.Keys.generate();
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

    container.read(agentUsageRelayProvider);
    await Future<void>.delayed(Duration.zero);

    relaySession.emit(
      _turnMetricEvent(
        ownerKeychain: otherOwnerKeychain,
        agentKeychain: agentKeychain,
        payload: {'harness': 'goose', 'timestamp': '2026-07-01T20:11:03Z'},
      ),
    );

    final state = container.read(agentUsageRelayProvider);
    expect(state.connection, AgentUsageConnectionState.open);
    expect(state.snapshotsByAgent, isEmpty);
    expect(state.errorMessage, isNull);
  });

  test(
    'a malformed/unparseable event is dropped silently, not surfaced as an error',
    () async {
      // NIP-AM Client Behavior: "ignore events that fail to decrypt or
      // parse" — a single bad event from one agent must not degrade the
      // usage view for every other agent.
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

      container.read(agentUsageRelayProvider);
      await Future<void>.delayed(Duration.zero);

      // Missing "harness" — fails AgentTurnMetricPayload.fromJson.
      relaySession.emit(
        _turnMetricEvent(
          ownerKeychain: ownerKeychain,
          agentKeychain: agentKeychain,
          payload: {'timestamp': '2026-07-01T20:11:03Z'},
        ),
      );

      final state = container.read(agentUsageRelayProvider);
      expect(state.connection, AgentUsageConnectionState.open);
      expect(state.errorMessage, isNull);
      expect(state.snapshotsByAgent, isEmpty);
    },
  );

  test(
    'a later event does not erase a field the newest event omitted',
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

      container.read(agentUsageRelayProvider);
      await Future<void>.delayed(Duration.zero);

      relaySession.emit(
        _turnMetricEvent(
          ownerKeychain: ownerKeychain,
          agentKeychain: agentKeychain,
          payload: {
            'harness': 'goose',
            'timestamp': '2026-07-01T20:00:00Z',
            'accountUsageWindows': [
              {'label': 'Session', 'usedPercent': 10.0},
            ],
          },
        ),
      );
      relaySession.emit(
        _turnMetricEvent(
          ownerKeychain: ownerKeychain,
          agentKeychain: agentKeychain,
          payload: {
            'harness': 'goose',
            'timestamp': '2026-07-01T20:05:00Z',
            'contextUsedTokens': 200,
            'contextLimitTokens': 1000,
          },
        ),
      );

      final snapshot = container
          .read(agentUsageRelayProvider)
          .snapshotsByAgent[agentKeychain.public];
      expect(snapshot!.contextUsedTokens, 200);
      expect(snapshot.accountUsageWindows.single.label, 'Session');
    },
  );

  test(
    'agentUsageSnapshotProvider exposes one agent from the shared relay state',
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

      container.read(agentUsageRelayProvider);
      await Future<void>.delayed(Duration.zero);

      expect(
        container.read(agentUsageSnapshotProvider(agentKeychain.public)),
        isNull,
      );

      relaySession.emit(
        _turnMetricEvent(
          ownerKeychain: ownerKeychain,
          agentKeychain: agentKeychain,
          payload: {
            'harness': 'goose',
            'timestamp': '2026-07-01T20:11:03Z',
            'contextUsedTokens': 10,
            'contextLimitTokens': 100,
          },
        ),
      );

      final snapshot = container.read(
        agentUsageSnapshotProvider(agentKeychain.public.toUpperCase()),
      );
      expect(snapshot?.contextUsedTokens, 10);
    },
  );
}

NostrEvent _turnMetricEvent({
  required nostr.Keys ownerKeychain,
  required nostr.Keys agentKeychain,
  required Map<String, dynamic> payload,
}) {
  final conversationKey = getConversationKey(
    agentKeychain.secret,
    ownerKeychain.public,
  );
  final event = nostr.Event.from(
    kind: EventKind.agentTurnMetric,
    content: nip44Encrypt(conversationKey, jsonEncode(payload)),
    tags: [
      ['p', ownerKeychain.public],
      ['agent', agentKeychain.public],
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
}

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  final String? _nsec;

  _FakeRelayConfigNotifier({required String? nsec}) : _nsec = nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'http://localhost:3000', nsec: _nsec);
}
