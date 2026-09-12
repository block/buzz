import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/shared/agent_usage/agent_usage_subscription.dart';
import 'package:buzz/shared/agent_usage/agent_usage_models.dart';
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

  test('identity switch clears trusted agents and history requests', () async {
    final firstOwner = nostr.Keys.generate();
    final secondOwner = nostr.Keys.generate();
    final agent = nostr.Keys.generate();
    final relaySession = _RecordingRelaySession();
    final relayConfig = _FakeRelayConfigNotifier(nsec: firstOwner.nsec);
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => relaySession),
        relayConfigProvider.overrideWith(() => relayConfig),
      ],
    );
    addTearDown(container.dispose);
    final subscription = container.listen(
      agentUsageRelayProvider,
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(subscription.close);

    await Future<void>.delayed(Duration.zero);
    await container
        .read(agentUsageRelayProvider.notifier)
        .ensureAgentHistory(agentPubkey: agent.public);
    expect(relaySession.historyFilters, hasLength(1));

    relayConfig.setIdentity(secondOwner.nsec);
    for (var attempt = 0; attempt < 20; attempt++) {
      await Future<void>.delayed(Duration.zero);
      if (relaySession.filters.length == 1 &&
          relaySession.filters.single.tags['#p']?.single ==
              secondOwner.public) {
        break;
      }
    }
    expect(relaySession.filters, hasLength(1));
    expect(relaySession.filters.single.tags['#p'], [secondOwner.public]);

    expect(
      relaySession.historyFilters,
      hasLength(1),
      reason: 'the previous identity history request must not be replayed',
    );
    relaySession.emit(
      _turnMetricEvent(
        ownerKeychain: secondOwner,
        agentKeychain: agent,
        payload: {
          'harness': 'test-harness',
          'timestamp': '2026-07-01T20:11:03Z',
        },
      ),
    );
    expect(
      container.read(agentUsageRelayProvider).snapshotsByAgent,
      isEmpty,
      reason: 'the previous identity agent must be trusted again explicitly',
    );
  });

  test('bounds remembered history scopes and evicts the oldest', () async {
    final ownerKeychain = nostr.Keys.generate();
    final agentKeychain = nostr.Keys.generate();
    final relaySession = _RecordingRelaySession(historyPages: const [[]]);
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
    for (var index = 0; index <= 256; index++) {
      await container
          .read(agentUsageRelayProvider.notifier)
          .ensureAgentHistory(
            agentPubkey: agentKeychain.public,
            channelId: index.toRadixString(16).padLeft(64, '0'),
          );
    }
    expect(relaySession.historyFilters, hasLength(257));

    await container
        .read(agentUsageRelayProvider.notifier)
        .ensureAgentHistory(
          agentPubkey: agentKeychain.public,
          channelId: 0.toRadixString(16).padLeft(64, '0'),
        );

    expect(relaySession.historyFilters, hasLength(258));
  });

  test(
    'replays a pre-registration event after the agent becomes trusted',
    () async {
      final ownerKeychain = nostr.Keys.generate();
      final agentKeychain = nostr.Keys.generate();
      final event = _turnMetricEvent(
        ownerKeychain: ownerKeychain,
        agentKeychain: agentKeychain,
        payload: {
          'v': 1,
          'harness': 'goose',
          'timestamp': '2026-07-01T12:00:00Z',
          'contextUsedTokens': 100,
          'contextLimitTokens': 200,
        },
      );
      final relaySession = _RecordingRelaySession(
        historyPages: [
          [event],
        ],
      );
      final relayConfig = _FakeRelayConfigNotifier(nsec: ownerKeychain.nsec);
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => relaySession),
          relayConfigProvider.overrideWith(() => relayConfig),
        ],
      );
      addTearDown(container.dispose);
      final subscription = container.listen(
        agentUsageRelayProvider,
        (_, _) {},
        fireImmediately: true,
      );
      addTearDown(subscription.close);
      await Future<void>.delayed(Duration.zero);

      relaySession.emit(event);
      await Future<void>.delayed(Duration.zero);
      expect(container.read(agentUsageRelayProvider).snapshotsByAgent, isEmpty);

      await container
          .read(agentUsageRelayProvider.notifier)
          .ensureAgentHistory(agentPubkey: agentKeychain.public);

      expect(
        container
            .read(agentUsageRelayProvider)
            .snapshotsByAgent[agentKeychain.public]
            ?.contextUsedTokens,
        100,
      );
    },
  );

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
    container
        .read(agentUsageRelayProvider.notifier)
        .trackAgent(agentKeychain.public);

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
    container
        .read(agentUsageRelayProvider.notifier)
        .trackAgent(agentKeychain.public);

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
    container
        .read(agentUsageRelayProvider.notifier)
        .trackAgent(agentKeychain.public);

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

  test('drops malformed NIP-AM envelopes before snapshot ingestion', () async {
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
    container
        .read(agentUsageRelayProvider.notifier)
        .trackAgent(agentKeychain.public);
    const payload = {'harness': 'goose', 'timestamp': '2026-07-01T20:11:03Z'};
    relaySession.emit(
      _turnMetricEvent(
        ownerKeychain: ownerKeychain,
        agentKeychain: agentKeychain,
        payload: payload,
        kind: EventKind.note,
      ),
    );
    relaySession.emit(
      _turnMetricEvent(
        ownerKeychain: ownerKeychain,
        agentKeychain: agentKeychain,
        payload: payload,
        extraTags: [
          ['p', ownerKeychain.public],
        ],
      ),
    );
    relaySession.emit(
      _turnMetricEvent(
        ownerKeychain: ownerKeychain,
        agentKeychain: agentKeychain,
        payload: payload,
        extraTags: [
          ['agent', agentKeychain.public],
        ],
      ),
    );
    relaySession.emit(
      _turnMetricEvent(
        ownerKeychain: ownerKeychain,
        agentKeychain: agentKeychain,
        payload: payload,
        extraTags: const [
          ['h', 'private-channel'],
        ],
      ),
    );

    expect(container.read(agentUsageRelayProvider).snapshotsByAgent, isEmpty);
  });

  test('bounds tracked agents and ignores an evicted relay author', () async {
    final ownerKeychain = nostr.Keys.generate();
    final oldestAgent = nostr.Keys.generate();
    final newestAgent = nostr.Keys.generate();
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
    final notifier = container.read(agentUsageRelayProvider.notifier);
    notifier.trackAgent(oldestAgent.public);
    for (var index = 1; index < 64; index++) {
      notifier.trackAgent(index.toRadixString(16).padLeft(64, '0'));
    }
    notifier.trackAgent(newestAgent.public);

    relaySession.emit(
      _turnMetricEvent(
        ownerKeychain: ownerKeychain,
        agentKeychain: oldestAgent,
        payload: const {
          'harness': 'goose',
          'timestamp': '2026-07-01T20:11:03Z',
          'contextUsedTokens': 10,
          'contextLimitTokens': 100,
        },
      ),
    );
    relaySession.emit(
      _turnMetricEvent(
        ownerKeychain: ownerKeychain,
        agentKeychain: newestAgent,
        payload: const {
          'harness': 'goose',
          'timestamp': '2026-07-01T20:12:03Z',
          'contextUsedTokens': 20,
          'contextLimitTokens': 100,
        },
      ),
    );

    final snapshots = container.read(agentUsageRelayProvider).snapshotsByAgent;
    expect(snapshots, hasLength(1));
    expect(snapshots, isNot(contains(oldestAgent.public)));
    expect(snapshots[newestAgent.public]?.contextUsedTokens, 20);
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
      container
          .read(agentUsageRelayProvider.notifier)
          .trackAgent(agentKeychain.public);

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
      container
          .read(agentUsageRelayProvider.notifier)
          .trackAgent(agentKeychain.public);

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
      container
          .read(agentUsageRelayProvider.notifier)
          .trackAgent(agentKeychain.public);

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
  test(
    'keeps the oldest second inclusive while draining a full history page',
    () async {
      final ownerKeychain = nostr.Keys.generate();
      final agentKeychain = nostr.Keys.generate();
      const createdAt = 1782936300;
      final quotaEvent = _turnMetricEvent(
        ownerKeychain: ownerKeychain,
        agentKeychain: agentKeychain,
        payload: {
          'harness': 'goose',
          'timestamp': '2026-07-01T20:05:00Z',
          'accountUsageWindows': [
            {'label': 'Session', 'usedPercent': 25},
          ],
        },
        createdAt: createdAt,
      );
      final contextEvent = _turnMetricEvent(
        ownerKeychain: ownerKeychain,
        agentKeychain: agentKeychain,
        payload: {
          'harness': 'goose',
          'channelId': 'channel-a',
          'timestamp': '2026-07-01T20:00:00Z',
          'contextUsedTokens': 100,
          'contextLimitTokens': 1000,
        },
        createdAt: createdAt,
      );
      final fillerEvents = List.generate(
        199,
        (index) => _turnMetricEvent(
          ownerKeychain: ownerKeychain,
          agentKeychain: agentKeychain,
          payload: {
            'harness': 'goose',
            'timestamp': '2026-07-01T20:04:00Z',
            'sessionId': 'session-a',
            'turnSeq': index,
            'cumulative': {'inputTokens': index + 1},
          },
          createdAt: createdAt,
        ),
      );
      final relaySession = _RecordingRelaySession(
        historyPages: [
          [quotaEvent, ...fillerEvents],
          [contextEvent],
        ],
      );
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
      await container
          .read(agentUsageRelayProvider.notifier)
          .ensureAgentHistory(
            agentPubkey: agentKeychain.public,
            channelId: 'channel-a',
          );

      expect(relaySession.historyFilters, hasLength(2));
      expect(relaySession.historyFilters.first.authors, [agentKeychain.public]);
      expect(relaySession.historyFilters.first.tags['#p'], [
        ownerKeychain.public,
      ]);
      expect(relaySession.historyFilters.last.until, createdAt);
      final snapshot = container
          .read(agentUsageRelayProvider)
          .snapshotFor(
            agentPubkey: agentKeychain.public,
            channelId: 'channel-a',
          );
      expect(snapshot?.contextUsedTokens, 100);
      expect(snapshot?.accountUsageWindows.single.usedPercent, 25);
    },
  );

  test(
    'does not complete history when a full boundary page makes no progress',
    () async {
      final ownerKeychain = nostr.Keys.generate();
      final agentKeychain = nostr.Keys.generate();
      final repeatedEvent = _turnMetricEvent(
        ownerKeychain: ownerKeychain,
        agentKeychain: agentKeychain,
        payload: {
          'harness': 'goose',
          'timestamp': '2026-07-01T20:05:00Z',
          'accountUsageWindows': [
            {'label': 'Session', 'usedPercent': 25},
          ],
        },
        createdAt: 1782936300,
      );
      final fullPage = List.filled(200, repeatedEvent);
      final relaySession = _RecordingRelaySession(
        historyPages: [fullPage, fullPage],
      );
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
      final notifier = container.read(agentUsageRelayProvider.notifier);
      await notifier.ensureAgentHistory(
        agentPubkey: agentKeychain.public,
        channelId: 'channel-a',
      );
      expect(relaySession.historyFilters, hasLength(2));

      await notifier.ensureAgentHistory(
        agentPubkey: agentKeychain.public,
        channelId: 'channel-a',
      );
      expect(relaySession.historyFilters, hasLength(3));
    },
  );

  test('combines global provider windows with exact scoped context', () {
    final global = AgentUsageSnapshot(
      harness: 'test-harness',
      lastEventAt: DateTime.utc(2026, 7, 1, 3),
      contextUsedTokens: 900,
      contextLimitTokens: 1000,
      contextSnapshotAt: DateTime.utc(2026, 7, 1, 3),
      accountUsageWindows: const [
        AgentUsageWindow(label: 'Weekly', usedPercent: 80),
      ],
      accountUsageWindowsAt: DateTime.utc(2026, 7, 1, 3),
    );
    final scoped = AgentUsageSnapshot(
      harness: 'test-harness',
      lastEventAt: DateTime.utc(2026, 7, 1, 1),
      contextUsedTokens: 100,
      contextLimitTokens: 1000,
      contextSnapshotAt: DateTime.utc(2026, 7, 1, 1),
    );
    final state = AgentUsageRelayState(
      connection: AgentUsageConnectionState.open,
      snapshotsByAgent: {'agent-a': global},
      scopedSnapshots: {
        (agentPubkey: 'agent-a', channelId: 'channel-a', threadRootId: null):
            scoped,
      },
    );

    final channelA = state.snapshotFor(
      agentPubkey: 'AGENT-A',
      channelId: 'channel-a',
    );
    final unknownChannel = state.snapshotFor(
      agentPubkey: 'agent-a',
      channelId: 'channel-b',
    );

    expect(channelA?.contextUsedTokens, 100);
    expect(channelA?.accountUsageWindows.single.usedPercent, 80);
    expect(unknownChannel?.contextUsedTokens, isNull);
    expect(unknownChannel?.accountUsageWindows.single.usedPercent, 80);
  });
}

NostrEvent _turnMetricEvent({
  required nostr.Keys ownerKeychain,
  required nostr.Keys agentKeychain,
  required Map<String, dynamic> payload,
  int kind = EventKind.agentTurnMetric,
  List<List<String>> extraTags = const [],
  int? createdAt,
}) {
  final conversationKey = getConversationKey(
    agentKeychain.secret,
    ownerKeychain.public,
  );
  final event = nostr.Event.from(
    kind: kind,
    content: nip44Encrypt(conversationKey, jsonEncode(payload)),
    createdAt: createdAt,
    tags: [
      ['p', ownerKeychain.public],
      ['agent', agentKeychain.public],
      ...extraTags,
    ],
    secretKey: agentKeychain.secret,
    verify: false,
  );
  return NostrEvent.fromJson(event.toMap());
}

class _RecordingRelaySession extends RelaySessionNotifier {
  final List<NostrFilter> filters = [];
  final List<NostrFilter> historyFilters = [];
  final List<List<NostrEvent>> historyPages;
  final List<void Function(NostrEvent)> _listeners = [];
  final List<void Function(String message)> _closedListeners = [];

  _RecordingRelaySession({List<List<NostrEvent>> historyPages = const []})
    : historyPages = List.of(historyPages);

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    historyFilters.add(filter);
    return historyPages.isEmpty ? const [] : historyPages.removeAt(0);
  }

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
  String? _nsec;

  _FakeRelayConfigNotifier({required String? nsec}) : _nsec = nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'http://localhost:3000', nsec: _nsec);

  void setIdentity(String? nsec) {
    _nsec = nsec;
    state = RelayConfig(baseUrl: 'http://localhost:3000', nsec: nsec);
  }
}
