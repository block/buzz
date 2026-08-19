import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/agent_activity/active_agent_turns.dart';
import 'package:buzz/features/channels/agent_activity/observer_models.dart';
import 'package:buzz/features/channels/agent_activity/observer_subscription.dart';
import 'package:buzz/features/channels/agent_activity/working_bots_provider.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channel_typing_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';

const _channelId = 'channel-1';

void main() {
  test('prefers observer work and keeps human typing separate', () {
    final observerTurn = _turn(
      'agent-a',
      livenessTimeout: const Duration(seconds: 40),
    );
    final container = ProviderContainer(
      overrides: [
        currentPubkeyProvider.overrideWith((ref) => 'owner'),
        channelMembersProvider(
          _channelId,
        ).overrideWith((ref) async => const <ChannelMember>[]),
        channelTypingProvider(_channelId).overrideWith(
          () => _FakeTypingNotifier([
            _typingEntry(
              'agent-a',
              receivedAt: DateTime.utc(2026, 8, 16, 12, 0, 9),
            ),
            _typingEntry('agent-b'),
            _typingEntry('human'),
          ]),
        ),
        agentMentionPubkeysProvider(
          _channelId,
        ).overrideWith((ref) => const {'agent-a', 'agent-b'}),
        agentOwnersProvider.overrideWithValue(
          const AsyncData({'agent-a': 'owner', 'agent-b': 'owner'}),
        ),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
        observerRelayProvider.overrideWith(
          () => _FakeObserverRelayNotifier({
            'agent-a': [_observerFrame('agent-a')],
          }),
        ),
        composerAgentTurnStatesProvider.overrideWithValue([observerTurn]),
      ],
    );
    addTearDown(container.dispose);

    final state = container.read(
      composerActivityStateProvider((
        channelId: _channelId,
        threadHeadId: null,
      )),
    );

    expect(state.agents, hasLength(2));
    expect(state.agents[0].pubkey, 'agent-a');
    expect(state.agents[0].source, AgentWorkingSource.observer);
    expect(state.agents[0].canViewActivity, isTrue);
    expect(state.agents[1].source, AgentWorkingSource.typing);
    expect(state.agents[1].canViewActivity, isTrue);
    expect(state.humanTyping.single.pubkey, 'human');
  });

  test('validated observer work survives unavailable agent lookups', () {
    final observerTurn = _turn('agent-a');
    final container = ProviderContainer(
      overrides: [
        currentPubkeyProvider.overrideWith((ref) => 'owner'),
        channelMembersProvider(
          _channelId,
        ).overrideWith((ref) => Future<List<ChannelMember>>.error('offline')),
        channelTypingProvider(
          _channelId,
        ).overrideWith(() => _FakeTypingNotifier(const [])),
        agentMentionPubkeysProvider(
          _channelId,
        ).overrideWith((ref) => const <String>{}),
        agentOwnersProvider.overrideWithValue(const AsyncLoading()),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
        observerRelayProvider.overrideWith(
          () => _FakeObserverRelayNotifier({
            'agent-a': [_observerFrame('agent-a')],
          }),
        ),
        composerAgentTurnStatesProvider.overrideWithValue([observerTurn]),
      ],
    );
    addTearDown(container.dispose);

    final state = container.read(
      composerActivityStateProvider((
        channelId: _channelId,
        threadHeadId: null,
      )),
    );

    expect(state.agents.single.pubkey, 'agent-a');
    expect(state.agents.single.source, AgentWorkingSource.observer);
    expect(state.agents.single.canViewActivity, isTrue);
  });

  test(
    'thread scope requires typing and does not surface observer-only work',
    () {
      final container = ProviderContainer(
        overrides: [
          currentPubkeyProvider.overrideWith((ref) => 'owner'),
          channelMembersProvider(
            _channelId,
          ).overrideWith((ref) async => const <ChannelMember>[]),
          channelTypingProvider(_channelId).overrideWith(
            () => _FakeTypingNotifier(const [
              TypingEntry(
                pubkey: 'agent-b',
                threadHeadId: 'thread-1',
                expiresAtMs: 9999999999999,
              ),
              TypingEntry(
                pubkey: 'human',
                threadHeadId: 'thread-1',
                expiresAtMs: 9999999999999,
              ),
            ]),
          ),
          agentMentionPubkeysProvider(
            _channelId,
          ).overrideWith((ref) => const {'agent-a', 'agent-b'}),
          agentOwnersProvider.overrideWithValue(
            const AsyncData({'agent-b': 'owner'}),
          ),
          userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
          observerRelayProvider.overrideWith(
            () => _FakeObserverRelayNotifier({
              'agent-a': [_observerFrame('agent-a')],
            }),
          ),
          composerAgentTurnStatesProvider.overrideWithValue([_turn('agent-a')]),
        ],
      );
      addTearDown(container.dispose);

      final state = container.read(
        composerActivityStateProvider((
          channelId: _channelId,
          threadHeadId: 'thread-1',
        )),
      );

      expect(state.agents.single.pubkey, 'agent-b');
      expect(state.agents.single.source, AgentWorkingSource.typing);
      expect(state.humanTyping.single.pubkey, 'human');
    },
  );

  for (final retainedThreadHeadId in <String?>[null, 'thread-1']) {
    test(
      'thread typing replaces a retained terminal turn '
      '${retainedThreadHeadId == null ? 'from another scope' : 'in the same scope'}',
      () {
        final failedTurn = _turn(
          'agent-a',
          phase: AgentTurnPhase.error,
          threadHeadId: retainedThreadHeadId,
        );
        final container = ProviderContainer(
          overrides: [
            currentPubkeyProvider.overrideWith((ref) => 'owner'),
            channelMembersProvider(
              _channelId,
            ).overrideWith((ref) async => const <ChannelMember>[]),
            channelTypingProvider(_channelId).overrideWith(
              () => _FakeTypingNotifier(const [
                TypingEntry(
                  pubkey: 'agent-a',
                  threadHeadId: 'thread-1',
                  expiresAtMs: 9999999999999,
                ),
              ]),
            ),
            agentMentionPubkeysProvider(
              _channelId,
            ).overrideWith((ref) => const {'agent-a'}),
            agentOwnersProvider.overrideWithValue(
              const AsyncData({'agent-a': 'owner'}),
            ),
            userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
            observerRelayProvider.overrideWith(
              () => _FakeObserverRelayNotifier({
                'agent-a': [_observerFrame('agent-a')],
              }),
            ),
            composerAgentTurnStatesProvider.overrideWithValue([failedTurn]),
          ],
        );
        addTearDown(container.dispose);

        final signal = container
            .read(
              composerActivityStateProvider((
                channelId: _channelId,
                threadHeadId: 'thread-1',
              )),
            )
            .agents
            .single;

        expect(signal.source, AgentWorkingSource.typing);
        expect(signal.isWorking, isTrue);
        expect(signal.turnId, isNull);
        expect(signal.startedAt, isNull);
        expect(container.read(workingBotPubkeysProvider(_channelId)), {
          'agent-a',
        });
      },
    );
  }

  test('same-turn typing preserves observer identity past a long cadence', () {
    final observerTurn = _turn(
      'agent-a',
      threadHeadId: 'thread-1',
      turnId: 'turn-1',
      lastActivityAt: DateTime.utc(2026, 8, 16, 12),
      livenessTimeout: const Duration(minutes: 2, seconds: 30),
    );
    final container = ProviderContainer(
      overrides: [
        currentPubkeyProvider.overrideWith((ref) => 'owner'),
        channelMembersProvider(
          _channelId,
        ).overrideWith((ref) async => const <ChannelMember>[]),
        channelTypingProvider(_channelId).overrideWith(
          () => _FakeTypingNotifier([
            _typingEntry(
              'agent-a',
              threadHeadId: 'thread-1',
              turnId: 'turn-1',
              receivedAt: DateTime.utc(2026, 8, 16, 12, 3),
            ),
          ]),
        ),
        agentMentionPubkeysProvider(
          _channelId,
        ).overrideWith((ref) => const {'agent-a'}),
        agentOwnersProvider.overrideWithValue(const AsyncData({})),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
        observerRelayProvider.overrideWith(
          () => _FakeObserverRelayNotifier({
            'agent-a': [_observerFrame('agent-a')],
          }),
        ),
        composerAgentTurnStatesProvider.overrideWithValue([observerTurn]),
      ],
    );
    addTearDown(container.dispose);

    final signal = container
        .read(
          composerActivityStateProvider((
            channelId: _channelId,
            threadHeadId: 'thread-1',
          )),
        )
        .agents
        .single;

    expect(signal.source, AgentWorkingSource.observer);
    expect(signal.turnId, 'turn-1');
    expect(signal.canViewActivity, isTrue);
  });

  test('fresh typing supersedes a stale working turn in the same scope', () {
    final staleTurn = _turn(
      'agent-a',
      threadHeadId: 'thread-1',
      lastActivityAt: DateTime.utc(2026, 8, 16, 12),
      livenessTimeout: const Duration(days: 7, seconds: 30),
    );
    final container = ProviderContainer(
      overrides: [
        currentPubkeyProvider.overrideWith((ref) => 'owner'),
        channelMembersProvider(
          _channelId,
        ).overrideWith((ref) async => const <ChannelMember>[]),
        channelTypingProvider(_channelId).overrideWith(
          () => _FakeTypingNotifier([
            _typingEntry(
              'agent-a',
              threadHeadId: 'thread-1',
              receivedAt: DateTime.utc(2026, 8, 16, 12, 0, 31),
            ),
          ]),
        ),
        agentMentionPubkeysProvider(
          _channelId,
        ).overrideWith((ref) => const {'agent-a'}),
        agentOwnersProvider.overrideWithValue(
          const AsyncData({'agent-a': 'owner'}),
        ),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
        observerRelayProvider.overrideWith(
          () => _FakeObserverRelayNotifier({
            'agent-a': [_observerFrame('agent-a')],
          }),
        ),
        composerAgentTurnStatesProvider.overrideWithValue([staleTurn]),
      ],
    );
    addTearDown(container.dispose);

    final signal = container
        .read(
          composerActivityStateProvider((
            channelId: _channelId,
            threadHeadId: 'thread-1',
          )),
        )
        .agents
        .single;

    expect(signal.source, AgentWorkingSource.typing);
    expect(signal.isWorking, isTrue);
    expect(signal.turnId, isNull);
    expect(signal.startedAt, isNull);
    expect(container.read(workingBotPubkeysProvider(_channelId)), {'agent-a'});
  });

  test(
    'keeps a scoped terminal outcome reachable after thread typing stops',
    () {
      final failedTurn = _turn(
        'agent-a',
        phase: AgentTurnPhase.error,
        threadHeadId: 'thread-1',
      );
      final container = ProviderContainer(
        overrides: [
          currentPubkeyProvider.overrideWith((ref) => 'owner'),
          channelMembersProvider(
            _channelId,
          ).overrideWith((ref) async => const <ChannelMember>[]),
          channelTypingProvider(
            _channelId,
          ).overrideWith(() => _FakeTypingNotifier(const [])),
          agentMentionPubkeysProvider(
            _channelId,
          ).overrideWith((ref) => const {'agent-a'}),
          agentOwnersProvider.overrideWithValue(
            const AsyncData({'agent-a': 'owner'}),
          ),
          userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
          observerRelayProvider.overrideWith(
            () => _FakeObserverRelayNotifier({
              'agent-a': [_observerFrame('agent-a')],
            }),
          ),
          composerAgentTurnStatesProvider.overrideWithValue([failedTurn]),
        ],
      );
      addTearDown(container.dispose);

      final signal = container
          .read(
            composerActivityStateProvider((
              channelId: _channelId,
              threadHeadId: 'thread-1',
            )),
          )
          .agents
          .single;

      expect(signal.source, AgentWorkingSource.observer);
      expect(signal.isWorking, isFalse);
      expect(signal.phase, AgentTurnPhase.error);
      expect(signal.turnId, failedTurn.turnId);
    },
  );

  test('keeps a recent owned error reachable after typing stops', () {
    final failedTurn = _turn('agent-a', phase: AgentTurnPhase.error);
    final container = ProviderContainer(
      overrides: [
        currentPubkeyProvider.overrideWith((ref) => 'owner'),
        channelMembersProvider(
          _channelId,
        ).overrideWith((ref) async => const <ChannelMember>[]),
        channelTypingProvider(
          _channelId,
        ).overrideWith(() => _FakeTypingNotifier(const [])),
        agentMentionPubkeysProvider(
          _channelId,
        ).overrideWith((ref) => const {'agent-a'}),
        agentOwnersProvider.overrideWithValue(
          const AsyncData({'agent-a': 'owner'}),
        ),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
        observerRelayProvider.overrideWith(
          () => _FakeObserverRelayNotifier({
            'agent-a': [_observerFrame('agent-a')],
          }),
        ),
        composerAgentTurnStatesProvider.overrideWithValue([failedTurn]),
      ],
    );
    addTearDown(container.dispose);

    final state = container.read(
      composerActivityStateProvider((
        channelId: _channelId,
        threadHeadId: null,
      )),
    );

    expect(state.agents, hasLength(1));
    expect(state.agents.single.pubkey, 'agent-a');
    expect(state.agents.single.turnId, failedTurn.turnId);
    expect(state.agents.single.canViewActivity, isTrue);
    expect(state.agents.single.isWorking, isFalse);
    expect(state.humanTyping, isEmpty);
    expect(container.read(workingBotPubkeysProvider(_channelId)), isEmpty);
  });

  test('channel working set includes agents from every thread scope', () {
    final container = ProviderContainer(
      overrides: [
        currentPubkeyProvider.overrideWith((ref) => 'owner'),
        channelMembersProvider(
          _channelId,
        ).overrideWith((ref) async => const <ChannelMember>[]),
        channelTypingProvider(_channelId).overrideWith(
          () => _FakeTypingNotifier(const [
            TypingEntry(
              pubkey: 'agent-a',
              threadHeadId: 'thread-1',
              expiresAtMs: 9999999999999,
            ),
            TypingEntry(
              pubkey: 'human',
              threadHeadId: 'thread-3',
              expiresAtMs: 9999999999999,
            ),
          ]),
        ),
        agentMentionPubkeysProvider(
          _channelId,
        ).overrideWith((ref) => const {'agent-a', 'agent-b'}),
        agentOwnersProvider.overrideWithValue(
          const AsyncData({'agent-a': 'owner', 'agent-b': 'owner'}),
        ),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
        observerRelayProvider.overrideWith(
          () => _FakeObserverRelayNotifier({
            'agent-b': [_observerFrame('agent-b')],
          }),
        ),
        composerAgentTurnStatesProvider.overrideWithValue([
          _turn('agent-b', threadHeadId: 'thread-2'),
        ]),
      ],
    );
    addTearDown(container.dispose);

    expect(container.read(workingBotPubkeysProvider(_channelId)), {
      'agent-a',
      'agent-b',
    });
  });

  test('prefers a newer terminal outcome over stale working history', () {
    final container = ProviderContainer(
      overrides: [
        currentPubkeyProvider.overrideWith((ref) => 'owner'),
        channelMembersProvider(
          _channelId,
        ).overrideWith((ref) async => const <ChannelMember>[]),
        channelTypingProvider(
          _channelId,
        ).overrideWith(() => _FakeTypingNotifier(const [])),
        agentMentionPubkeysProvider(
          _channelId,
        ).overrideWith((ref) => const {'agent-a'}),
        agentOwnersProvider.overrideWithValue(
          const AsyncData({'agent-a': 'owner'}),
        ),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
        observerRelayProvider.overrideWith(
          () => _FakeObserverRelayNotifier({
            'agent-a': [_observerFrame('agent-a')],
          }),
        ),
        composerAgentTurnStatesProvider.overrideWithValue([
          _turn(
            'agent-a',
            turnId: 'turn-old',
            startedAt: DateTime.utc(2026, 8, 16, 11, 59),
            lastActivityAt: DateTime.utc(2026, 8, 16, 12, 0, 1),
          ),
          _turn(
            'agent-a',
            phase: AgentTurnPhase.error,
            turnId: 'turn-new',
            startedAt: DateTime.utc(2026, 8, 16, 12),
            lastActivityAt: DateTime.utc(2026, 8, 16, 12, 0, 5),
          ),
        ]),
      ],
    );
    addTearDown(container.dispose);

    final state = container.read(
      composerActivityStateProvider((
        channelId: _channelId,
        threadHeadId: null,
      )),
    );

    expect(state.agents.single.turnId, 'turn-new');
    expect(state.agents.single.phase, AgentTurnPhase.error);
    expect(state.agents.single.isWorking, isFalse);
    expect(container.read(workingBotPubkeysProvider(_channelId)), isEmpty);
  });

  test('prefers live work when turn chronology is tied', () {
    final receiptAt = DateTime.utc(2026, 8, 16, 12, 0, 5);
    final startedAt = DateTime.utc(2026, 8, 16, 12, 0, 4);
    final container = ProviderContainer(
      overrides: [
        currentPubkeyProvider.overrideWith((ref) => 'owner'),
        channelMembersProvider(
          _channelId,
        ).overrideWith((ref) async => const <ChannelMember>[]),
        channelTypingProvider(
          _channelId,
        ).overrideWith(() => _FakeTypingNotifier(const [])),
        agentMentionPubkeysProvider(
          _channelId,
        ).overrideWith((ref) => const {'agent-a'}),
        agentOwnersProvider.overrideWithValue(
          const AsyncData({'agent-a': 'owner'}),
        ),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
        observerRelayProvider.overrideWith(
          () => _FakeObserverRelayNotifier({
            'agent-a': [_observerFrame('agent-a')],
          }),
        ),
        composerAgentTurnStatesProvider.overrideWithValue([
          _turn(
            'agent-a',
            phase: AgentTurnPhase.error,
            turnId: 'turn-old',
            startedAt: startedAt,
            lastActivityAt: receiptAt,
          ),
          _turn(
            'agent-a',
            turnId: 'turn-new',
            startedAt: startedAt,
            lastActivityAt: receiptAt,
          ),
        ]),
      ],
    );
    addTearDown(container.dispose);

    final state = container.read(
      composerActivityStateProvider((
        channelId: _channelId,
        threadHeadId: null,
      )),
    );

    expect(state.agents.single.turnId, 'turn-new');
    expect(state.agents.single.isWorking, isTrue);
    expect(container.read(workingBotPubkeysProvider(_channelId)), {'agent-a'});
  });
}

AgentTurnState _turn(
  String pubkey, {
  AgentTurnPhase phase = AgentTurnPhase.working,
  String? turnId,
  String? threadHeadId,
  DateTime? startedAt,
  DateTime? lastActivityAt,
  Duration livenessTimeout = const Duration(seconds: 30),
}) => AgentTurnState(
  agentPubkey: pubkey,
  channelId: _channelId,
  threadHeadId: threadHeadId,
  turnId: turnId ?? 'turn-$pubkey',
  startedAt: startedAt ?? DateTime.utc(2026, 8, 16, 12),
  lastActivityAt: lastActivityAt ?? DateTime.utc(2026, 8, 16, 12),
  livenessTimeout: livenessTimeout,
  phase: phase,
  terminalAt: phase == AgentTurnPhase.working
      ? null
      : DateTime.utc(2026, 8, 16, 12, 0, 5),
);

TypingEntry _typingEntry(
  String pubkey, {
  String? threadHeadId,
  String? turnId,
  DateTime? receivedAt,
}) {
  final received = receivedAt ?? DateTime.utc(2026, 8, 16, 12);
  return TypingEntry(
    pubkey: pubkey,
    threadHeadId: threadHeadId,
    turnId: turnId,
    expiresAtMs: received.add(TypingEntry.ttl).millisecondsSinceEpoch,
  );
}

ObserverFrame _observerFrame(String pubkey) => ObserverFrame(
  seq: 1,
  timestamp: DateTime.utc(2026, 8, 16, 12).toIso8601String(),
  kind: 'turn_started',
  channelId: _channelId,
  turnId: 'turn-$pubkey',
);

class _FakeTypingNotifier extends ChannelTypingNotifier {
  final List<TypingEntry> entries;

  _FakeTypingNotifier(this.entries) : super(_channelId);

  @override
  List<TypingEntry> build() => entries;
}

class _FakeUserCacheNotifier extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => const {};

  @override
  void preload(List<String> pubkeys) {}
}

class _FakeObserverRelayNotifier extends ObserverRelayNotifier {
  final Map<String, List<ObserverFrame>> frames;

  _FakeObserverRelayNotifier(this.frames);

  @override
  ObserverRelayState build() => ObserverRelayState(
    connection: ObserverConnectionState.open,
    framesByAgent: frames,
  );
}
